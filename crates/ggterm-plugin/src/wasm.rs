//! WASM plugin runtime — wasmtime-based sandboxed execution.
//!
//! WASM plugins run in a sandboxed environment with zero host access by default.
//! The host provides a limited set of import functions for logging and context
//! access, following a capability-based security model.
//!
//! ## WASM Plugin ABI
//!
//! ### Required Exports
//!
//! The WASM module must export:
//!
//! - `memory` — linear memory for string passing
//! - `gg_name() -> i32` — returns pointer to null-terminated name string
//! - `gg_version() -> i32` — returns pointer to null-terminated version string
//! - `gg_hooks() -> i32` — returns bitmask of registered hooks
//! - `gg_init() -> ()` — called once on plugin initialization
//! - `gg_shutdown() -> ()` — called on plugin shutdown
//! - `gg_handle_hook(type: i32, data_ptr: i32, data_len: i32) -> i32` — dispatch hook
//! - `gg_get_result(ptr: i32, max_len: i32) -> i32` — read result text after Transform
//!
//! ### Hook Type IDs
//!
//! | ID | Hook |
//! |----|------|
//! | 0  | OnInput |
//! | 1  | OnOutput |
//! | 2  | OnCommandStart |
//! | 3  | OnCommandEnd |
//! | 4  | OnResize |
//! | 5  | OnThemeChange |
//!
//! ### Result Codes from `gg_handle_hook`
//!
//! | Code | Meaning |
//! |------|---------|
//! | 0    | Allow |
//! | 1    | Deny |
//! | 2    | Transform (call `gg_get_result` to read new text) |
//! | 3    | Annotate (key=value from `gg_get_result`, split on `\0`) |
//!
//! ### Hook Data Format
//!
//! Hook data is written as UTF-8 text into WASM linear memory before calling
//! `gg_handle_hook`. The format depends on hook type:
//!
//! - **OnInput/OnOutput/OnCommandStart**: raw text content
//! - **OnCommandEnd**: `command\0exit_code` (null-separated)
//! - **OnResize**: `cols,rows`
//! - **OnThemeChange**: `from\0to`
//!
//! ### Host Imports (optional)
//!
//! The WASM module may import from the `ggterm` module:
//!
//! - `gg_log(level: i32, ptr: i32, len: i32)` — log a message

use std::sync::Mutex;

use crate::hooks::{Hook, HookResult, HookType};
use crate::plugin::{Plugin, PluginContext, PluginError, PluginStats};

// ── Hook type IDs ──────────────────────────────────────────────────

const HOOK_ON_INPUT: i32 = 0;
const HOOK_ON_OUTPUT: i32 = 1;
const HOOK_ON_COMMAND_START: i32 = 2;
const HOOK_ON_COMMAND_END: i32 = 3;
const HOOK_ON_RESIZE: i32 = 4;
const HOOK_ON_THEME_CHANGE: i32 = 5;

const HOOK_BIT_ON_INPUT: i32 = 1 << HOOK_ON_INPUT;
const HOOK_BIT_ON_OUTPUT: i32 = 1 << HOOK_ON_OUTPUT;
const HOOK_BIT_ON_COMMAND_START: i32 = 1 << HOOK_ON_COMMAND_START;
const HOOK_BIT_ON_COMMAND_END: i32 = 1 << HOOK_ON_COMMAND_END;
const HOOK_BIT_ON_RESIZE: i32 = 1 << HOOK_ON_RESIZE;
const HOOK_BIT_ON_THEME_CHANGE: i32 = 1 << HOOK_ON_THEME_CHANGE;

fn hook_type_to_id(ht: HookType) -> i32 {
    match ht {
        HookType::OnInput => HOOK_ON_INPUT,
        HookType::OnOutput => HOOK_ON_OUTPUT,
        HookType::OnCommandStart => HOOK_ON_COMMAND_START,
        HookType::OnCommandEnd => HOOK_ON_COMMAND_END,
        HookType::OnResize => HOOK_ON_RESIZE,
        HookType::OnThemeChange => HOOK_ON_THEME_CHANGE,
    }
}

fn id_to_hook_type(id: i32) -> Option<HookType> {
    match id {
        HOOK_ON_INPUT => Some(HookType::OnInput),
        HOOK_ON_OUTPUT => Some(HookType::OnOutput),
        HOOK_ON_COMMAND_START => Some(HookType::OnCommandStart),
        HOOK_ON_COMMAND_END => Some(HookType::OnCommandEnd),
        HOOK_ON_RESIZE => Some(HookType::OnResize),
        HOOK_ON_THEME_CHANGE => Some(HookType::OnThemeChange),
        _ => None,
    }
}

fn bitmask_to_hooks(mask: i32) -> Vec<HookType> {
    let mut hooks = Vec::new();
    for id in 0..6 {
        if mask & (1 << id) != 0 {
            if let Some(ht) = id_to_hook_type(id) {
                hooks.push(ht);
            }
        }
    }
    hooks
}

// ── Result codes ───────────────────────────────────────────────────

const RESULT_ALLOW: i32 = 0;
const RESULT_DENY: i32 = 1;
const RESULT_TRANSFORM: i32 = 2;
const RESULT_ANNOTATE: i32 = 3;

// ── Host state for wasmtime Store ──────────────────────────────────

/// Host state available inside WASM imports.
#[derive(Default)]
struct HostState {
    /// Accumulated log messages for debugging.
    log_messages: Vec<(i32, String)>,
}

// ── WasmPlugin ─────────────────────────────────────────────────────

/// A WASM-based plugin loaded via wasmtime.
///
/// Implements the [`Plugin`] trait so it can be registered with
/// [`PluginManager`](crate::manager::PluginManager) alongside native and Lua plugins.
///
/// The WASM module runs in a sandbox with no filesystem, network, or
/// environment access. Only the host-provided import functions are available.
pub struct WasmPlugin {
    name: String,
    version: String,
    registered_hooks: Vec<HookType>,
    stats: PluginStats,
    initialized: bool,
    // Opaque engine/module/store — boxed to keep wasmtime types private
    // from the rest of the crate.
    inner: Mutex<WasmInner>,
}

struct WasmInner {
    #[allow(dead_code)]
    engine: wasmtime::Engine,
    module: wasmtime::Module,
    store: wasmtime::Store<HostState>,
    memory: wasmtime::Memory,
}

impl WasmInner {
    /// Write a byte slice into WASM linear memory at the given offset.
    fn write_memory(&mut self, offset: usize, data: &[u8]) -> Result<(), PluginError> {
        self.memory
            .data_mut(&mut self.store)
            .get_mut(offset..offset + data.len())
            .ok_or_else(|| PluginError::Wasm("memory write out of bounds".into()))?
            .copy_from_slice(data);
        Ok(())
    }

    /// Read a null-terminated string from WASM linear memory.
    fn read_cstr(&self, offset: usize) -> Result<String, PluginError> {
        let mem = self.memory.data(&self.store);
        let end = mem[offset..]
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| PluginError::Wasm("unterminated string in memory".into()))?;
        let bytes = &mem[offset..offset + end];
        String::from_utf8(bytes.to_vec())
            .map_err(|e| PluginError::Wasm(format!("invalid UTF-8 in string: {e}")))
    }

    /// Read `len` bytes from WASM linear memory as a string.
    fn read_string(&self, offset: usize, len: usize) -> Result<String, PluginError> {
        let mem = self.memory.data(&self.store);
        let bytes = mem
            .get(offset..offset + len)
            .ok_or_else(|| PluginError::Wasm("memory read out of bounds".into()))?;
        String::from_utf8(bytes.to_vec())
            .map_err(|e| PluginError::Wasm(format!("invalid UTF-8: {e}")))
    }

    /// Grow WASM linear memory to ensure at least `required` bytes are available.
    fn ensure_capacity(&mut self, required: usize) -> Result<(), PluginError> {
        let current = self.memory.data_size(&self.store);
        if current < required {
            let pages_needed =
                ((required - current) + wasmtime::WASM_PAGE_SIZE - 1) / wasmtime::WASM_PAGE_SIZE;
            self.memory
                .grow(&mut self.store, pages_needed as u64)
                .map_err(|e| PluginError::Wasm(format!("failed to grow memory: {e}")))?;
        }
        Ok(())
    }
}

impl WasmPlugin {
    /// Load a WASM plugin from a `.wasm` file.
    pub fn from_file(path: impl AsRef<std::path::Path>) -> Result<Self, PluginError> {
        let wasm_bytes = std::fs::read(path.as_ref()).map_err(|e| {
            PluginError::Wasm(format!("failed to read {:?}: {e}", path.as_ref()))
        })?;
        Self::from_bytes(&wasm_bytes)
    }

    /// Load a WASM plugin from raw bytes.
    pub fn from_bytes(wasm_bytes: &[u8]) -> Result<Self, PluginError> {
        // Configure engine: Cranelift compiler, no parallel compilation (smaller)
        let engine = wasmtime::Engine::new(
            &wasmtime::Config::new().strategy(wasmtime::Strategy::Cranelift),
        )
        .map_err(|e| PluginError::Wasm(format!("failed to create engine: {e}")))?;

        let module = wasmtime::Module::new(&engine, wasm_bytes)
            .map_err(|e| PluginError::Wasm(format!("failed to compile WASM: {e}")))?;

        // Set up host imports
        let mut host_state = HostState::default();

        // Create the gg_log import function
        let log_func = wasmtime::Func::new(
            &engine,
            wasmtime::FuncType::new(
                &[
                    wasmtime::ValType::I32,
                    wasmtime::ValType::I32,
                    wasmtime::ValType::I32,
                ],
                &[],
            ),
            {
                let state_ptr = &mut host_state as *mut HostState;
                move |mut caller, params, _results| {
                    let level = params[0].unwrap_i32();
                    let ptr = params[1].unwrap_i32() as usize;
                    let len = params[2].unwrap_i32() as usize;

                    // SAFETY: caller is single-threaded, state_ptr is valid
                    let state = unsafe { &mut *state_ptr };
                    if let Some(mem) = caller.get_export("memory") {
                        if let Some(memory) = mem.into_memory() {
                            let data = memory.data(&caller);
                            if let Some(slice) = data.get(ptr..ptr + len) {
                                if let Ok(msg) = String::from_utf8(slice.to_vec()) {
                                    state.log_messages.push((level, msg));
                                }
                            }
                        }
                    }
                    Ok(())
                }
            },
        );

        let mut linker = wasmtime::Linker::new(&engine);
        linker
            .define("ggterm", "gg_log", log_func)
            .map_err(|e| PluginError::Wasm(format!("failed to define import: {e}")))?;

        let mut store = wasmtime::Store::new(&engine, host_state);

        // Instantiate with linker (provides host imports)
        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| PluginError::Wasm(format!("failed to instantiate WASM: {e}")))?;

        // Get memory export
        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| PluginError::Wasm("missing 'memory' export".into()))?;

        let inner = WasmInner {
            engine,
            module,
            store,
            memory,
        };

        let mut plugin = Self {
            name: String::new(),
            version: String::new(),
            registered_hooks: Vec::new(),
            stats: PluginStats::default(),
            initialized: false,
            inner: Mutex::new(inner),
        };

        // Call gg_name, gg_version, gg_hooks to populate metadata
        plugin.read_metadata()?;

        Ok(plugin)
    }

    /// Call the WASM module's `gg_name`, `gg_version`, and `gg_hooks` exports
    /// to populate the plugin's metadata.
    fn read_metadata(&mut self) -> Result<(), PluginError> {
        let mut inner = self.inner.lock().unwrap();

        // Get name
        if let Some(func) = inner.module.get_export(None, "gg_name").and_then(|e| {
            e.into_func().and_then(|f| {
                f.typed::<(), i32>(&inner.engine)
                    .ok()
            })
        }) {
            let ptr = func
                .call(&mut inner.store, ())
                .map_err(|e| PluginError::Wasm(format!("gg_name failed: {e}")))?;
            if ptr > 0 {
                inner.name_str_from_ptr(ptr as usize, &mut self.name)?;
            }
        }

        // Get version
        if let Some(func) = inner.module.get_export(None, "gg_version").and_then(|e| {
            e.into_func().and_then(|f| {
                f.typed::<(), i32>(&inner.engine)
                    .ok()
            })
        }) {
            let ptr = func
                .call(&mut inner.store, ())
                .map_err(|e| PluginError::Wasm(format!("gg_version failed: {e}")))?;
            if ptr > 0 {
                inner.version_str_from_ptr(ptr as usize, &mut self.version)?;
            }
        }

        // Get hooks bitmask
        if let Some(func) = inner.module.get_export(None, "gg_hooks").and_then(|e| {
            e.into_func().and_then(|f| {
                f.typed::<(), i32>(&inner.engine)
                    .ok()
            })
        }) {
            let mask = func
                .call(&mut inner.store, ())
                .map_err(|e| PluginError::Wasm(format!("gg_hooks failed: {e}")))?;
            self.registered_hooks = bitmask_to_hooks(mask);
        }

        // If name is still empty, use a fallback
        if self.name.is_empty() {
            self.name = "wasm-plugin".to_string();
        }
        if self.version.is_empty() {
            self.version = "0.1.0".to_string();
        }
        if self.registered_hooks.is_empty() {
            // Default to all hooks if the module doesn't declare them
            self.registered_hooks = HookType::all().to_vec();
        }

        Ok(())
    }

    /// Get accumulated log messages (for debugging).
    pub fn take_log_messages(&self) -> Vec<(i32, String)> {
        let mut inner = self.inner.lock().unwrap();
        std::mem::take(&mut inner.store.data_mut().log_messages)
    }

    /// Get the runtime stats.
    pub fn stats(&self) -> &PluginStats {
        &self.stats
    }
}

/// Extension methods on WasmInner for reading strings.
impl WasmInner {
    fn name_str_from_ptr(&self, ptr: usize, out: &mut String) -> Result<(), PluginError> {
        *out = self.read_cstr(ptr)?;
        Ok(())
    }

    fn version_str_from_ptr(&self, ptr: usize, out: &mut String) -> Result<(), PluginError> {
        *out = self.read_cstr(ptr)?;
        Ok(())
    }
}

impl Plugin for WasmPlugin {
    fn name(&self) -> &str {
        &self.name
    }

    fn version(&self) -> &str {
        &self.version
    }

    fn init(&mut self, _ctx: &PluginContext) -> Result<(), PluginError> {
        if self.initialized {
            return Ok(());
        }

        let mut inner = self.inner.lock().unwrap();

        if let Some(func) = inner.module.get_export(None, "gg_init").and_then(|e| {
            e.into_func().and_then(|f| {
                f.typed::<(), ()>(&inner.engine)
                    .ok()
            })
        }) {
            func.call(&mut inner.store, ())
                .map_err(|e| PluginError::Wasm(format!("gg_init failed: {e}")))?;
        }

        drop(inner);
        self.initialized = true;
        Ok(())
    }

    fn shutdown(&mut self) {
        let mut inner = self.inner.lock().unwrap();

        if let Some(func) = inner.module.get_export(None, "gg_shutdown").and_then(|e| {
            e.into_func().and_then(|f| {
                f.typed::<(), ()>(&inner.engine)
                    .ok()
            })
        }) {
            let _ = func.call(&mut inner.store, ());
        }

        self.initialized = false;
    }

    fn hooks(&self) -> &[HookType] {
        &self.registered_hooks
    }

    fn handle_hook(&mut self, hook: &Hook, _ctx: &PluginContext) -> HookResult {
        // Serialize hook data for WASM
        let (hook_type_id, data) = serialize_hook(hook);

        let mut inner = self.inner.lock().unwrap();

        // Ensure WASM memory has space for the data
        // We write at a fixed scratch offset (0) — the module must not use
        // offset 0 for persistent data.
        const SCRATCH_OFFSET: usize = 1024; // Start at 1KB to avoid null-page

        if let Err(e) = inner.ensure_capacity(SCRATCH_OFFSET + data.len() + 1) {
            self.stats.record_error();
            log::warn!("WASM plugin '{}' memory error: {}", self.name, e);
            return HookResult::Allow;
        }

        // Write hook data into WASM memory
        if let Err(e) = inner.write_memory(SCRATCH_OFFSET, &data) {
            self.stats.record_error();
            log::warn!("WASM plugin '{}' write error: {}", self.name, e);
            return HookResult::Allow;
        }

        // Call gg_handle_hook(type, data_ptr, data_len)
        let result_code = if let Some(func) =
            inner.module.get_export(None, "gg_handle_hook").and_then(|e| {
                e.into_func().and_then(|f| {
                    f.typed::<(i32, i32, i32), i32>(&inner.engine).ok()
                })
            })
        {
            match func.call(
                &mut inner.store,
                (hook_type_id, SCRATCH_OFFSET as i32, data.len() as i32),
            ) {
                Ok(code) => code,
                Err(e) => {
                    self.stats.record_error();
                    log::warn!("WASM plugin '{}' hook error: {}", self.name, e);
                    return HookResult::Allow;
                }
            }
        } else {
            // No handler exported — default to Allow
            return HookResult::Allow;
        };

        // Convert result code to HookResult
        let result = match result_code {
            RESULT_ALLOW => HookResult::Allow,
            RESULT_DENY => HookResult::Deny,
            RESULT_TRANSFORM => {
                // Call gg_get_result to read transformed text
                if let Some(func) = inner
                    .module
                    .get_export(None, "gg_get_result")
                    .and_then(|e| {
                        e.into_func().and_then(|f| {
                            f.typed::<(i32, i32), i32>(&inner.engine).ok()
                        })
                    })
                {
                    // Write into a result buffer at a different scratch area
                    const RESULT_BUF_OFFSET: usize = 4096;
                    const RESULT_BUF_MAX: usize = 8192;

                    let _ = inner.ensure_capacity(RESULT_BUF_OFFSET + RESULT_BUF_MAX);

                    let actual_len = func
                        .call(
                            &mut inner.store,
                            (RESULT_BUF_OFFSET as i32, RESULT_BUF_MAX as i32),
                        )
                        .unwrap_or(0);

                    if actual_len > 0 && actual_len as usize <= RESULT_BUF_MAX {
                        match inner.read_string(RESULT_BUF_OFFSET, actual_len as usize) {
                            Ok(text) => HookResult::Transform(text),
                            Err(_) => HookResult::Allow,
                        }
                    } else {
                        HookResult::Allow
                    }
                } else {
                    HookResult::Allow
                }
            }
            RESULT_ANNOTATE => {
                // Call gg_get_result for annotation data "key\0value"
                if let Some(func) = inner
                    .module
                    .get_export(None, "gg_get_result")
                    .and_then(|e| {
                        e.into_func().and_then(|f| {
                            f.typed::<(i32, i32), i32>(&inner.engine).ok()
                        })
                    })
                {
                    const RESULT_BUF_OFFSET: usize = 4096;
                    const RESULT_BUF_MAX: usize = 8192;

                    let _ = inner.ensure_capacity(RESULT_BUF_OFFSET + RESULT_BUF_MAX);

                    let actual_len = func
                        .call(
                            &mut inner.store,
                            (RESULT_BUF_OFFSET as i32, RESULT_BUF_MAX as i32),
                        )
                        .unwrap_or(0);

                    if actual_len > 0 && actual_len as usize <= RESULT_BUF_MAX {
                        match inner.read_string(RESULT_BUF_OFFSET, actual_len as usize) {
                            Ok(text) => {
                                let parts: Vec<&str> = text.splitn(2, '\0').collect();
                                if parts.len() == 2 {
                                    HookResult::Annotate(
                                        parts[0].to_string(),
                                        parts[1].to_string(),
                                    )
                                } else {
                                    HookResult::Allow
                                }
                            }
                            Err(_) => HookResult::Allow,
                        }
                    } else {
                        HookResult::Allow
                    }
                } else {
                    HookResult::Allow
                }
            }
            _ => HookResult::Allow,
        };

        self.stats.record(&result);
        result
    }
}

/// Serialize a Hook into (type_id, data_bytes) for WASM consumption.
fn serialize_hook(hook: &Hook) -> (i32, Vec<u8>) {
    match hook {
        Hook::OnInput(text) => (HOOK_ON_INPUT, text.as_bytes().to_vec()),
        Hook::OnOutput(text) => (HOOK_ON_OUTPUT, text.as_bytes().to_vec()),
        Hook::OnCommandStart(cmd) => (HOOK_ON_COMMAND_START, cmd.as_bytes().to_vec()),
        Hook::OnCommandEnd { command, exit_code } => {
            (HOOK_ON_COMMAND_END, format!("{command}\0{exit_code}").into_bytes())
        }
        Hook::OnResize { cols, rows } => {
            (HOOK_ON_RESIZE, format!("{cols},{rows}").into_bytes())
        }
        Hook::OnThemeChange { from, to } => {
            (HOOK_ON_THEME_CHANGE, format!("{from}\0{to}").into_bytes())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Hook type ID conversion tests ──────────────────────────────

    #[test]
    fn t_hook_type_to_id_roundtrip() {
        for ht in HookType::all() {
            let id = hook_type_to_id(*ht);
            assert_eq!(id_to_hook_type(id), Some(*ht));
        }
    }

    #[test]
    fn t_hook_type_ids_are_sequential() {
        assert_eq!(hook_type_to_id(HookType::OnInput), 0);
        assert_eq!(hook_type_to_id(HookType::OnOutput), 1);
        assert_eq!(hook_type_to_id(HookType::OnCommandStart), 2);
        assert_eq!(hook_type_to_id(HookType::OnCommandEnd), 3);
        assert_eq!(hook_type_to_id(HookType::OnResize), 4);
        assert_eq!(hook_type_to_id(HookType::OnThemeChange), 5);
    }

    #[test]
    fn t_id_to_hook_type_invalid() {
        assert_eq!(id_to_hook_type(99), None);
        assert_eq!(id_to_hook_type(-1), None);
    }

    // ── Bitmask conversion tests ───────────────────────────────────

    #[test]
    fn t_bitmask_empty() {
        assert!(bitmask_to_hooks(0).is_empty());
    }

    #[test]
    fn t_bitmask_single() {
        let hooks = bitmask_to_hooks(HOOK_BIT_ON_INPUT);
        assert_eq!(hooks, vec![HookType::OnInput]);
    }

    #[test]
    fn t_bitmask_multiple() {
        let mask = HOOK_BIT_ON_INPUT | HOOK_BIT_ON_OUTPUT | HOOK_BIT_ON_COMMAND_END;
        let hooks = bitmask_to_hooks(mask);
        assert_eq!(hooks.len(), 3);
        assert!(hooks.contains(&HookType::OnInput));
        assert!(hooks.contains(&HookType::OnOutput));
        assert!(hooks.contains(&HookType::OnCommandEnd));
    }

    #[test]
    fn t_bitmask_all() {
        let mask = HOOK_BIT_ON_INPUT
            | HOOK_BIT_ON_OUTPUT
            | HOOK_BIT_ON_COMMAND_START
            | HOOK_BIT_ON_COMMAND_END
            | HOOK_BIT_ON_RESIZE
            | HOOK_BIT_ON_THEME_CHANGE;
        let hooks = bitmask_to_hooks(mask);
        assert_eq!(hooks.len(), 6);
    }

    // ── Hook serialization tests ───────────────────────────────────

    #[test]
    fn t_serialize_on_input() {
        let (id, data) = serialize_hook(&Hook::OnInput("ls -la".into()));
        assert_eq!(id, HOOK_ON_INPUT);
        assert_eq!(data, b"ls -la");
    }

    #[test]
    fn t_serialize_on_output() {
        let (id, data) = serialize_hook(&Hook::OnOutput("hello\n".into()));
        assert_eq!(id, HOOK_ON_OUTPUT);
        assert_eq!(data, b"hello\n");
    }

    #[test]
    fn t_serialize_on_command_start() {
        let (id, data) = serialize_hook(&Hook::OnCommandStart("git status".into()));
        assert_eq!(id, HOOK_ON_COMMAND_START);
        assert_eq!(data, b"git status");
    }

    #[test]
    fn t_serialize_on_command_end() {
        let (id, data) = serialize_hook(&Hook::OnCommandEnd {
            command: "make".into(),
            exit_code: 127,
        });
        assert_eq!(id, HOOK_ON_COMMAND_END);
        assert_eq!(data, b"make\0127");
    }

    #[test]
    fn t_serialize_on_resize() {
        let (id, data) = serialize_hook(&Hook::OnResize {
            cols: 120,
            rows: 40,
        });
        assert_eq!(id, HOOK_ON_RESIZE);
        assert_eq!(data, b"120,40");
    }

    #[test]
    fn t_serialize_on_theme_change() {
        let (id, data) = serialize_hook(&Hook::OnThemeChange {
            from: "dark".into(),
            to: "dracula".into(),
        });
        assert_eq!(id, HOOK_ON_THEME_CHANGE);
        assert_eq!(data, b"dark\0dracula");
    }

    // ── WasmPlugin with minimal WASM module ────────────────────────
    //
    // We build a tiny WASM module in WAT (WebAssembly Text) format
    // that exports the required functions.

    /// WAT source for a minimal valid WASM plugin.
    const MINIMAL_PLUGIN_WAT: &str = r#"
(module
  ;; Linear memory
  (memory (export "memory") 1)

  ;; Plugin name: "test-plugin\0" at offset 0
  (data (i32.const 0) "test-plugin\00")

  ;; Plugin version: "1.0.0\0" at offset 64
  (data (i32.const 64) "1.0.0\00")

  ;; Return name pointer
  (func (export "gg_name") (result i32)
    i32.const 0)

  ;; Return version pointer
  (func (export "gg_version") (result i32)
    i32.const 64)

  ;; Register for OnInput (bit 0) only
  (func (export "gg_hooks") (result i32)
    i32.const 1)

  ;; Init — no-op
  (func (export "gg_init"))

  ;; Shutdown — no-op
  (func (export "gg_shutdown"))

  ;; Handle hook — always returns Allow (0)
  (func (export "gg_handle_hook")
    (param i32 i32 i32) (result i32)
    i32.const 0)

  ;; Get result — returns 0 (no text)
  (func (export "gg_get_result")
    (param i32 i32) (result i32)
    i32.const 0)
)
"#;

    /// WAT source for a plugin that transforms input to uppercase.
    const UPPERCASE_PLUGIN_WAT: &str = r#"
(module
  ;; Linear memory (2 pages = 128KB)
  (memory (export "memory") 2)

  ;; Plugin name at offset 0
  (data (i32.const 0) "upper\00")
  ;; Plugin version at offset 64
  (data (i32.const 64) "0.1.0\00")

  ;; Scratch buffer for result text at offset 8192
  (global $result_len (mut i32) (i32.const 0))

  (func (export "gg_name") (result i32)
    i32.const 0)

  (func (export "gg_version") (result i32)
    i32.const 64)

  ;; Register for OnInput (bit 0)
  (func (export "gg_hooks") (result i32)
    i32.const 1)

  (func (export "gg_init"))
  (func (export "gg_shutdown"))

  ;; Handle hook: transform input to uppercase
  ;; data_ptr and data_len are params 1 and 2
  (func (export "gg_handle_hook")
    (param $hook_type i32) (param $data_ptr i32) (param $data_len i32)
    (result i32)

    ;; Copy input to result buffer (offset 8192), uppercasing ASCII
    (local $i i32)
    (local.set $i (i32.const 0))

    (block $done
      (loop $loop
        (br_if $done (i32.ge_s (local.get $i) (local.get $data_len)))

        ;; Load byte from input
        (local $byte i32)
        (local.set $byte
          (i32.load8_u
            (i32.add (local.get $data_ptr) (local.get $i))))

        ;; If lowercase a-z (97-122), subtract 32
        (if (i32.and
              (i32.ge_s (local.get $byte) (i32.const 97))
              (i32.le_s (local.get $byte) (i32.const 122)))
          (then
            (local.set $byte
              (i32.sub (local.get $byte) (i32.const 32)))))

        ;; Store to result buffer at offset 8192
        (i32.store8
          (i32.add (i32.const 8192) (local.get $i))
          (local.get $byte))

        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)
      )
    )

    ;; Save result length
    (global.set $result_len (local.get $data_len))

    ;; Return Transform (2)
    i32.const 2)

  ;; Get result: copy from internal buffer to caller's buffer
  (func (export "gg_get_result")
    (param $out_ptr i32) (param $max_len i32)
    (result i32)

    (local $len i32)
    (local.set $len (global.get $result_len))

    ;; Clamp to max_len
    (if (i32.gt_s (local.get $len) (local.get $max_len))
      (then (local.set $len (local.get $max_len))))

    ;; Copy bytes
    (local $i i32)
    (local.set $i (i32.const 0))
    (block $done
      (loop $loop
        (br_if $done (i32.ge_s (local.get $i) (local.get $len)))
        (i32.store8
          (i32.add (local.get $out_ptr) (local.get $i))
          (i32.load8_u
            (i32.add (i32.const 8192) (local.get $i))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)
      )
    )

    (local.get $len))
)
"#;

    /// WAT source for a plugin that denies all input.
    const DENY_PLUGIN_WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (data (i32.const 0) "denyer\00")
  (data (i32.const 64) "1.0.0\00")

  (func (export "gg_name") (result i32) i32.const 0)
  (func (export "gg_version") (result i32) i32.const 64)
  (func (export "gg_hooks") (result i32) i32.const 1)  ;; OnInput
  (func (export "gg_init"))
  (func (export "gg_shutdown"))

  ;; Always return Deny (1)
  (func (export "gg_handle_hook")
    (param i32 i32 i32) (result i32)
    i32.const 1)

  (func (export "gg_get_result")
    (param i32 i32) (result i32)
    i32.const 0)
)
"#;

    fn compile_wat(wat_src: &str) -> Vec<u8> {
        wat::parse_str(wat_src).unwrap()
    }

    // ── Minimal plugin tests ───────────────────────────────────────

    #[test]
    fn t_wasm_plugin_from_wat_minimal() {
        let wasm = compile_wat(MINIMAL_PLUGIN_WAT);
        let plugin = WasmPlugin::from_bytes(&wasm).unwrap();

        assert_eq!(plugin.name(), "test-plugin");
        assert_eq!(plugin.version(), "1.0.0");
    }

    #[test]
    fn t_wasm_plugin_hooks_minimal() {
        let wasm = compile_wat(MINIMAL_PLUGIN_WAT);
        let plugin = WasmPlugin::from_bytes(&wasm).unwrap();

        assert_eq!(plugin.hooks(), &[HookType::OnInput]);
    }

    #[test]
    fn t_wasm_plugin_init_minimal() {
        let wasm = compile_wat(MINIMAL_PLUGIN_WAT);
        let mut plugin = WasmPlugin::from_bytes(&wasm).unwrap();

        let ctx = PluginContext::default();
        assert!(plugin.init(&ctx).is_ok());
    }

    #[test]
    fn t_wasm_plugin_handle_hook_allow() {
        let wasm = compile_wat(MINIMAL_PLUGIN_WAT);
        let mut plugin = WasmPlugin::from_bytes(&wasm).unwrap();

        let ctx = PluginContext::default();
        plugin.init(&ctx).unwrap();

        let result = plugin.handle_hook(&Hook::OnInput("hello".into()), &ctx);
        assert_eq!(result, HookResult::Allow);
    }

    #[test]
    fn t_wasm_plugin_shutdown() {
        let wasm = compile_wat(MINIMAL_PLUGIN_WAT);
        let mut plugin = WasmPlugin::from_bytes(&wasm).unwrap();

        let ctx = PluginContext::default();
        plugin.init(&ctx).unwrap();
        plugin.shutdown();
        // shutdown is idempotent
        plugin.shutdown();
    }

    #[test]
    fn t_wasm_plugin_double_init() {
        let wasm = compile_wat(MINIMAL_PLUGIN_WAT);
        let mut plugin = WasmPlugin::from_bytes(&wasm).unwrap();

        let ctx = PluginContext::default();
        plugin.init(&ctx).unwrap();
        // Second init should be a no-op
        assert!(plugin.init(&ctx).is_ok());
    }

    // ── Transform plugin tests ─────────────────────────────────────

    #[test]
    fn t_wasm_plugin_uppercase_transform() {
        let wasm = compile_wat(UPPERCASE_PLUGIN_WAT);
        let mut plugin = WasmPlugin::from_bytes(&wasm).unwrap();

        assert_eq!(plugin.name(), "upper");

        let ctx = PluginContext::default();
        plugin.init(&ctx).unwrap();

        let result = plugin.handle_hook(&Hook::OnInput("hello world".into()), &ctx);
        match result {
            HookResult::Transform(text) => assert_eq!(text, "HELLO WORLD"),
            other => panic!("expected Transform, got {:?}", other),
        }
    }

    #[test]
    fn t_wasm_plugin_uppercase_empty() {
        let wasm = compile_wat(UPPERCASE_PLUGIN_WAT);
        let mut plugin = WasmPlugin::from_bytes(&wasm).unwrap();

        let ctx = PluginContext::default();
        plugin.init(&ctx).unwrap();

        let result = plugin.handle_hook(&Hook::OnInput("".into()), &ctx);
        match result {
            HookResult::Transform(text) => assert_eq!(text, ""),
            other => panic!("expected Transform, got {:?}", other),
        }
    }

    #[test]
    fn t_wasm_plugin_uppercase_already_upper() {
        let wasm = compile_wat(UPPERCASE_PLUGIN_WAT);
        let mut plugin = WasmPlugin::from_bytes(&wasm).unwrap();

        let ctx = PluginContext::default();
        plugin.init(&ctx).unwrap();

        let result = plugin.handle_hook(&Hook::OnInput("HELLO".into()), &ctx);
        match result {
            HookResult::Transform(text) => assert_eq!(text, "HELLO"),
            other => panic!("expected Transform, got {:?}", other),
        }
    }

    // ── Deny plugin tests ──────────────────────────────────────────

    #[test]
    fn t_wasm_plugin_deny_input() {
        let wasm = compile_wat(DENY_PLUGIN_WAT);
        let mut plugin = WasmPlugin::from_bytes(&wasm).unwrap();

        assert_eq!(plugin.name(), "denyer");

        let ctx = PluginContext::default();
        plugin.init(&ctx).unwrap();

        let result = plugin.handle_hook(&Hook::OnInput("rm -rf /".into()), &ctx);
        assert_eq!(result, HookResult::Deny);
    }

    // ── Stats tests ────────────────────────────────────────────────

    #[test]
    fn t_wasm_plugin_stats_after_allow() {
        let wasm = compile_wat(MINIMAL_PLUGIN_WAT);
        let mut plugin = WasmPlugin::from_bytes(&wasm).unwrap();

        let ctx = PluginContext::default();
        plugin.init(&ctx).unwrap();

        plugin.handle_hook(&Hook::OnInput("test".into()), &ctx);
        assert_eq!(plugin.stats().hooks_called, 1);
        assert_eq!(plugin.stats().denials, 0);
    }

    #[test]
    fn t_wasm_plugin_stats_after_deny() {
        let wasm = compile_wat(DENY_PLUGIN_WAT);
        let mut plugin = WasmPlugin::from_bytes(&wasm).unwrap();

        let ctx = PluginContext::default();
        plugin.init(&ctx).unwrap();

        plugin.handle_hook(&Hook::OnInput("test".into()), &ctx);
        assert_eq!(plugin.stats().hooks_called, 1);
        assert_eq!(plugin.stats().denials, 1);
    }

    #[test]
    fn t_wasm_plugin_stats_after_transform() {
        let wasm = compile_wat(UPPERCASE_PLUGIN_WAT);
        let mut plugin = WasmPlugin::from_bytes(&wasm).unwrap();

        let ctx = PluginContext::default();
        plugin.init(&ctx).unwrap();

        plugin.handle_hook(&Hook::OnInput("test".into()), &ctx);
        assert_eq!(plugin.stats().hooks_called, 1);
        assert_eq!(plugin.stats().transforms, 1);
    }

    // ── Error handling tests ───────────────────────────────────────

    #[test]
    fn t_wasm_plugin_invalid_wasm() {
        let result = WasmPlugin::from_bytes(b"not valid wasm");
        assert!(result.is_err());
    }

    #[test]
    fn t_wasm_plugin_missing_memory() {
        let wat = r#"
(module
  (func (export "gg_name") (result i32) i32.const 0)
  (func (export "gg_version") (result i32) i32.const 64)
  (func (export "gg_hooks") (result i32) i32.const 0)
)
"#;
        let wasm = compile_wat(wat);
        let result = WasmPlugin::from_bytes(&wasm);
        assert!(result.is_err());
    }

    // ── Plugin trait object test ───────────────────────────────────

    #[test]
    fn t_wasm_plugin_as_trait_object() {
        let wasm = compile_wat(MINIMAL_PLUGIN_WAT);
        let plugin = WasmPlugin::from_bytes(&wasm).unwrap();

        let boxed: Box<dyn Plugin> = Box::new(plugin);
        assert_eq!(boxed.name(), "test-plugin");
        assert_eq!(boxed.version(), "1.0.0");
    }
}
