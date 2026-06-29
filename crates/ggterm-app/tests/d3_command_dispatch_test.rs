//! P6-D3: OSC 133 command block dispatch integration tests.
//!
//! These tests verify that the PluginBridge correctly dispatches
//! OnCommandStart and OnCommandEnd hooks, and that the dispatch
//! integrates properly with the plugin manager lifecycle.

#![cfg(feature = "plugin")]

use std::sync::{Arc, Mutex};

use ggterm_app::plugin_integration::{PluginBridge, build_context};
use ggterm_plugin::{Hook, HookResult, HookType, NativePlugin};

/// A recording plugin that captures OnCommandStart / OnCommandEnd
/// calls into a shared Vec<String>.
fn recording_plugin(name: &str, hooks: &[HookType], log: Arc<Mutex<Vec<String>>>) -> NativePlugin {
    let l = log.clone();
    let mut builder = NativePlugin::new(name).version("1.0.0");
    for &h in hooks {
        builder = builder.hook(h);
    }
    builder
        .on_hook(move |hook, _ctx| {
            let entry = match hook {
                Hook::OnCommandStart(cmd) => format!("start|{cmd}"),
                Hook::OnCommandEnd { command, exit_code } => {
                    format!("end|{command}|{exit_code}")
                }
                _ => return HookResult::Allow,
            };
            l.lock().unwrap().push(entry);
            HookResult::Allow
        })
        .build()
}

// ── OnCommandStart dispatch ──

#[test]
fn t_d3_command_start_basic() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut bridge = PluginBridge::new();
    bridge
        .register(Box::new(recording_plugin(
            "rec",
            &[HookType::OnCommandStart],
            log.clone(),
        )))
        .unwrap();

    let ctx = build_context(80, 24, "dark");
    bridge.init_all(&ctx).unwrap();
    bridge.dispatch_command_start("ls -la", &ctx);

    let entries = log.lock().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0], "start|ls -la");
}

#[test]
fn t_d3_command_start_empty_command() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut bridge = PluginBridge::new();
    bridge
        .register(Box::new(recording_plugin(
            "rec",
            &[HookType::OnCommandStart],
            log.clone(),
        )))
        .unwrap();

    let ctx = build_context(80, 24, "dark");
    bridge.init_all(&ctx).unwrap();
    bridge.dispatch_command_start("", &ctx);

    let entries = log.lock().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0], "start|");
}

// ── OnCommandEnd dispatch ──

#[test]
fn t_d3_command_end_basic() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut bridge = PluginBridge::new();
    bridge
        .register(Box::new(recording_plugin(
            "rec",
            &[HookType::OnCommandEnd],
            log.clone(),
        )))
        .unwrap();

    let ctx = build_context(80, 24, "dark");
    bridge.init_all(&ctx).unwrap();
    bridge.dispatch_command_end("make test", 0, &ctx);

    let entries = log.lock().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0], "end|make test|0");
}

#[test]
fn t_d3_command_end_nonzero_exit() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut bridge = PluginBridge::new();
    bridge
        .register(Box::new(recording_plugin(
            "rec",
            &[HookType::OnCommandEnd],
            log.clone(),
        )))
        .unwrap();

    let ctx = build_context(80, 24, "dark");
    bridge.init_all(&ctx).unwrap();
    bridge.dispatch_command_end("false", 1, &ctx);
    bridge.dispatch_command_end("exit", 127, &ctx);

    let entries = log.lock().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0], "end|false|1");
    assert_eq!(entries[1], "end|exit|127");
}

#[test]
fn t_d3_command_end_negative_exit_code() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut bridge = PluginBridge::new();
    bridge
        .register(Box::new(recording_plugin(
            "rec",
            &[HookType::OnCommandEnd],
            log.clone(),
        )))
        .unwrap();

    let ctx = build_context(80, 24, "dark");
    bridge.init_all(&ctx).unwrap();
    bridge.dispatch_command_end("kill -TERM $$", -1, &ctx);

    let entries = log.lock().unwrap();
    assert_eq!(entries[0], "end|kill -TERM $$|-1");
}

// ── Combined lifecycle ──

#[test]
fn t_d3_command_lifecycle() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut bridge = PluginBridge::new();
    bridge
        .register(Box::new(recording_plugin(
            "rec",
            &[HookType::OnCommandStart, HookType::OnCommandEnd],
            log.clone(),
        )))
        .unwrap();

    let ctx = build_context(80, 24, "dark");
    bridge.init_all(&ctx).unwrap();

    // Simulate a full command lifecycle
    bridge.dispatch_command_start("git status", &ctx);
    bridge.dispatch_command_end("git status", 0, &ctx);

    // Another command
    bridge.dispatch_command_start("git diff", &ctx);
    bridge.dispatch_command_end("git diff", 1, &ctx);

    let entries = log.lock().unwrap();
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[0], "start|git status");
    assert_eq!(entries[1], "end|git status|0");
    assert_eq!(entries[2], "start|git diff");
    assert_eq!(entries[3], "end|git diff|1");
}

// ── Multiple plugins ──

#[test]
fn t_d3_multiple_plugins_all_receive_dispatch() {
    let log1 = Arc::new(Mutex::new(Vec::new()));
    let log2 = Arc::new(Mutex::new(Vec::new()));

    let mut bridge = PluginBridge::new();
    bridge
        .register(Box::new(recording_plugin(
            "p1",
            &[HookType::OnCommandStart],
            log1.clone(),
        )))
        .unwrap();
    bridge
        .register(Box::new(recording_plugin(
            "p2",
            &[HookType::OnCommandStart],
            log2.clone(),
        )))
        .unwrap();

    let ctx = build_context(80, 24, "dark");
    bridge.init_all(&ctx).unwrap();
    bridge.dispatch_command_start("cargo build", &ctx);

    assert_eq!(log1.lock().unwrap().len(), 1);
    assert_eq!(log2.lock().unwrap().len(), 1);
    assert_eq!(log1.lock().unwrap()[0], "start|cargo build");
    assert_eq!(log2.lock().unwrap()[0], "start|cargo build");
}

// ── Edge cases ──

#[test]
fn t_d3_no_panic_without_plugins() {
    let mut bridge = PluginBridge::new();
    let ctx = build_context(80, 24, "dark");
    // No plugins registered — should not panic
    bridge.dispatch_command_start("ls", &ctx);
    bridge.dispatch_command_end("ls", 0, &ctx);
}

#[test]
fn t_d3_plugin_not_subscribed_to_command_hooks() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut bridge = PluginBridge::new();
    // Plugin only subscribed to OnOutput
    bridge
        .register(Box::new(recording_plugin(
            "rec",
            &[HookType::OnOutput],
            log.clone(),
        )))
        .unwrap();

    let ctx = build_context(80, 24, "dark");
    bridge.init_all(&ctx).unwrap();
    bridge.dispatch_command_start("ls", &ctx);
    bridge.dispatch_command_end("ls", 0, &ctx);

    assert!(log.lock().unwrap().is_empty());
}

#[test]
fn t_d3_disabled_plugin_skipped() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut bridge = PluginBridge::new();
    bridge
        .register(Box::new(recording_plugin(
            "rec",
            &[HookType::OnCommandStart, HookType::OnCommandEnd],
            log.clone(),
        )))
        .unwrap();

    let ctx = build_context(80, 24, "dark");
    bridge.init_all(&ctx).unwrap();
    bridge.disable("rec");

    bridge.dispatch_command_start("ls", &ctx);
    bridge.dispatch_command_end("ls", 0, &ctx);

    assert!(log.lock().unwrap().is_empty());
}

#[test]
fn t_d3_stats_updated_on_dispatch() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut bridge = PluginBridge::new();
    bridge
        .register(Box::new(recording_plugin(
            "rec",
            &[HookType::OnCommandStart, HookType::OnCommandEnd],
            log.clone(),
        )))
        .unwrap();

    let ctx = build_context(80, 24, "dark");
    bridge.init_all(&ctx).unwrap();

    bridge.dispatch_command_start("cmd1", &ctx);
    bridge.dispatch_command_end("cmd1", 0, &ctx);
    bridge.dispatch_command_start("cmd2", &ctx);
    bridge.dispatch_command_end("cmd2", 1, &ctx);

    let stats = bridge.manager().stats_of("rec").unwrap();
    assert_eq!(stats.hooks_called, 4);
}

#[test]
fn t_d3_from_manager_preserves_plugins() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut mgr = ggterm_plugin::PluginManager::new();
    mgr.register(Box::new(recording_plugin(
        "native",
        &[HookType::OnCommandStart],
        log.clone(),
    )))
    .unwrap();

    let bridge = PluginBridge::from_manager(mgr);
    assert_eq!(bridge.count(), 1);

    let ctx = build_context(80, 24, "dark");
    let mut bridge = bridge;
    bridge.init_all(&ctx).unwrap();
    bridge.dispatch_command_start("echo hi", &ctx);

    assert_eq!(log.lock().unwrap().len(), 1);
}
