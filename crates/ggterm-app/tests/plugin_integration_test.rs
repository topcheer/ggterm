//! Plugin integration tests for PluginBridge.
//!
//! These tests exercise PluginBridge end-to-end with NativePlugin and
//! LuaPlugin, covering hook dispatch, multi-plugin aggregation, lifecycle,
//! and all hook types.

#![cfg(feature = "plugin")]

use ggterm_app::plugin_integration::{PluginBridge, build_context};
use ggterm_plugin::{Hook, HookResult, HookType, NativePlugin, native};

// ─── Test helpers ───

fn init_bridge(bridge: &mut PluginBridge) {
    let ctx = build_context(80, 24, "dark");
    let _ = bridge.init_all(&ctx);
}

/// Create a plugin that counts hook calls.
fn counter_plugin(
    name: &str,
    hook_type: HookType,
) -> (NativePlugin, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let c = count.clone();
    let plugin = native(name)
        .version("1.0.0")
        .hook(hook_type)
        .on_hook(move |h, _| {
            if h.hook_type() == hook_type {
                c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            HookResult::Allow
        })
        .build();
    (plugin, count)
}

// ═════════════════════════════════════════
// 1. NativePlugin dispatch — Allow / Deny / Transform / Annotate
// ═════════════════════════════════════════

#[test]
fn t_native_allow_input() {
    let mut bridge = PluginBridge::new();
    let p = native("allow-plugin")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .on_hook(|_, _| HookResult::Allow)
        .build();
    let _ = bridge.register(Box::new(p));
    init_bridge(&mut bridge);

    let ctx = build_context(80, 24, "dark");
    let result = bridge.dispatch_input("ls -la", &ctx);
    assert_eq!(result, HookResult::Allow);
}

#[test]
fn t_native_deny_input() {
    let mut bridge = PluginBridge::new();
    let p = native("deny-plugin")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .on_hook(|_, _| HookResult::Deny)
        .build();
    let _ = bridge.register(Box::new(p));
    init_bridge(&mut bridge);

    let ctx = build_context(80, 24, "dark");
    let result = bridge.dispatch_input("rm -rf /", &ctx);
    assert_eq!(result, HookResult::Deny);
}

#[test]
fn t_native_transform_input() {
    let mut bridge = PluginBridge::new();
    let p = native("transform-plugin")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .on_hook(|hook, _| {
            if let Hook::OnInput(text) = hook {
                let upper = text.to_uppercase();
                HookResult::Transform(upper)
            } else {
                HookResult::Allow
            }
        })
        .build();
    let _ = bridge.register(Box::new(p));
    init_bridge(&mut bridge);

    let ctx = build_context(80, 24, "dark");
    let result = bridge.dispatch_input("hello world", &ctx);
    assert_eq!(result, HookResult::Transform("HELLO WORLD".to_string()));
}

#[test]
fn t_native_annotate_input() {
    let mut bridge = PluginBridge::new();
    let p = native("annotate-plugin")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .on_hook(|_, _| HookResult::Annotate("risk".to_string(), "low".to_string()))
        .build();
    let _ = bridge.register(Box::new(p));
    init_bridge(&mut bridge);

    let ctx = build_context(80, 24, "dark");
    // dispatch_input returns aggregated result — Annotate is absorbed
    let result = bridge.dispatch_input("ls", &ctx);
    // Aggregator: Annotate → Allow (since finalize returns Allow if no Deny/Transform)
    assert_eq!(result, HookResult::Allow);
}

#[test]
fn t_native_transformed_text_extraction() {
    let mut bridge = PluginBridge::new();
    let p = native("prefix-plugin")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .on_hook(|_, _| HookResult::Transform("echo transformed".to_string()))
        .build();
    let _ = bridge.register(Box::new(p));
    init_bridge(&mut bridge);

    let ctx = build_context(80, 24, "dark");
    let result = bridge.dispatch_input("anything", &ctx);
    assert!(result.is_transform());
    assert_eq!(result.transformed_text(), Some("echo transformed"));
}

// ═════════════════════════════════════════
// 2. Multi-plugin hook aggregation
// ═════════════════════════════════════════

#[test]
fn t_deny_wins_over_allow() {
    let mut bridge = PluginBridge::new();
    let allow_p = native("allower")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .on_hook(|_, _| HookResult::Allow)
        .build();
    let deny_p = native("denier")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .on_hook(|_, _| HookResult::Deny)
        .build();
    let _ = bridge.register(Box::new(allow_p));
    let _ = bridge.register(Box::new(deny_p));
    init_bridge(&mut bridge);

    let ctx = build_context(80, 24, "dark");
    let result = bridge.dispatch_input("test", &ctx);
    assert_eq!(result, HookResult::Deny);
}

#[test]
fn t_deny_wins_over_transform() {
    let mut bridge = PluginBridge::new();
    let transform_p = native("transformer")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .on_hook(|_, _| HookResult::Transform("safe-command".to_string()))
        .build();
    let deny_p = native("denier")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .on_hook(|_, _| HookResult::Deny)
        .build();
    let _ = bridge.register(Box::new(transform_p));
    let _ = bridge.register(Box::new(deny_p));
    init_bridge(&mut bridge);

    let ctx = build_context(80, 24, "dark");
    let result = bridge.dispatch_input("dangerous", &ctx);
    assert_eq!(result, HookResult::Deny);
}

#[test]
fn t_last_transform_wins() {
    let mut bridge = PluginBridge::new();
    let first = native("first")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .on_hook(|_, _| HookResult::Transform("AAA".to_string()))
        .build();
    let second = native("second")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .on_hook(|_, _| HookResult::Transform("BBB".to_string()))
        .build();
    let _ = bridge.register(Box::new(first));
    let _ = bridge.register(Box::new(second));
    init_bridge(&mut bridge);

    let ctx = build_context(80, 24, "dark");
    let result = bridge.dispatch_input("test", &ctx);
    // Last Transform wins
    assert_eq!(result, HookResult::Transform("BBB".to_string()));
}

#[test]
fn t_disabled_plugin_skipped() {
    let mut bridge = PluginBridge::new();
    let deny_p = native("denier")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .on_hook(|_, _| HookResult::Deny)
        .build();
    let _ = bridge.register(Box::new(deny_p));
    init_bridge(&mut bridge);

    // Disable the plugin
    bridge.disable("denier");

    let ctx = build_context(80, 24, "dark");
    let result = bridge.dispatch_input("test", &ctx);
    // Disabled plugin → no Deny → Allow
    assert_eq!(result, HookResult::Allow);
}

#[test]
fn t_reenable_plugin_works() {
    let mut bridge = PluginBridge::new();
    let deny_p = native("denier")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .on_hook(|_, _| HookResult::Deny)
        .build();
    let _ = bridge.register(Box::new(deny_p));
    init_bridge(&mut bridge);

    bridge.disable("denier");
    let ctx = build_context(80, 24, "dark");
    let result = bridge.dispatch_input("test", &ctx);
    assert_eq!(result, HookResult::Allow);

    bridge.enable("denier");
    let result = bridge.dispatch_input("test", &ctx);
    assert_eq!(result, HookResult::Deny);
}

// ═════════════════════════════════════════
// 3. Plugin lifecycle
// ═════════════════════════════════════════

#[test]
fn t_lifecycle_init_called() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let initialized = Arc::new(AtomicBool::new(false));
    let init_flag = initialized.clone();

    let p = native("init-plugin")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .on_init(move |_| {
            init_flag.store(true, Ordering::SeqCst);
            Ok(())
        })
        .build();

    let mut bridge = PluginBridge::new();
    let _ = bridge.register(Box::new(p));

    let ctx = build_context(80, 24, "dark");
    let _ = bridge.init_all(&ctx);

    assert!(initialized.load(Ordering::SeqCst));
}

#[test]
fn t_lifecycle_dispatch_before_init_returns_allow() {
    let mut bridge = PluginBridge::new();
    let deny_p = native("denier")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .on_hook(|_, _| HookResult::Deny)
        .build();
    let _ = bridge.register(Box::new(deny_p));

    // Don't call init_all — plugin state is Stopped, not Active
    let ctx = build_context(80, 24, "dark");
    let result = bridge.dispatch_input("test", &ctx);
    // Plugin not Active → skipped → Allow
    assert_eq!(result, HookResult::Allow);
}

#[test]
fn t_lifecycle_shutdown_idempotent() {
    let mut bridge = PluginBridge::new();
    let p = native("simple")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .build();
    let _ = bridge.register(Box::new(p));
    init_bridge(&mut bridge);

    // Multiple shutdowns should not panic
    bridge.shutdown_all();
    bridge.shutdown_all();
    bridge.shutdown_all();
}

#[test]
fn t_lifecycle_unregister_after_init() {
    let mut bridge = PluginBridge::new();
    let deny_p = native("denier")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .on_hook(|_, _| HookResult::Deny)
        .build();
    let _ = bridge.register(Box::new(deny_p));
    init_bridge(&mut bridge);

    let ctx = build_context(80, 24, "dark");
    let result = bridge.dispatch_input("test", &ctx);
    assert_eq!(result, HookResult::Deny);

    bridge.unregister("denier");
    let result = bridge.dispatch_input("test", &ctx);
    assert_eq!(result, HookResult::Allow);
}

// ═════════════════════════════════════════
// 4. All hook dispatch types
// ═════════════════════════════════════════

#[test]
fn t_dispatch_output_no_panic() {
    let mut bridge = PluginBridge::new();
    let (p, count) = counter_plugin("output-plugin", HookType::OnOutput);
    let _ = bridge.register(Box::new(p));
    init_bridge(&mut bridge);

    let ctx = build_context(80, 24, "dark");
    bridge.dispatch_output("total 0\ndrwxr-xr-x", &ctx);
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn t_dispatch_resize_no_panic() {
    let mut bridge = PluginBridge::new();
    let (p, count) = counter_plugin("resize-plugin", HookType::OnResize);
    let _ = bridge.register(Box::new(p));
    init_bridge(&mut bridge);

    let ctx = build_context(80, 24, "dark");
    bridge.dispatch_resize(120, 40, &ctx);
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn t_dispatch_theme_change_no_panic() {
    let mut bridge = PluginBridge::new();
    let (p, count) = counter_plugin("theme-plugin", HookType::OnThemeChange);
    let _ = bridge.register(Box::new(p));
    init_bridge(&mut bridge);

    let ctx = build_context(80, 24, "dark");
    bridge.dispatch_theme_change("dark", "light", &ctx);
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn t_dispatch_command_start_no_panic() {
    let mut bridge = PluginBridge::new();
    let (p, count) = counter_plugin("cmd-plugin", HookType::OnCommandStart);
    let _ = bridge.register(Box::new(p));
    init_bridge(&mut bridge);

    let ctx = build_context(80, 24, "dark");
    bridge.dispatch_command_start("ls -la", &ctx);
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn t_dispatch_command_end_no_panic() {
    let mut bridge = PluginBridge::new();
    let (p, count) = counter_plugin("cmdend-plugin", HookType::OnCommandEnd);
    let _ = bridge.register(Box::new(p));
    init_bridge(&mut bridge);

    let ctx = build_context(80, 24, "dark");
    bridge.dispatch_command_end("ls -la", 0, &ctx);
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn t_multiple_hook_types_single_plugin() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let input_count = Arc::new(AtomicUsize::new(0));
    let output_count = Arc::new(AtomicUsize::new(0));
    let ic = input_count.clone();
    let oc = output_count.clone();

    let p = native("multi-hook")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .hook(HookType::OnOutput)
        .on_hook(move |hook, _| {
            match hook.hook_type() {
                HookType::OnInput => {
                    ic.fetch_add(1, Ordering::SeqCst);
                }
                HookType::OnOutput => {
                    oc.fetch_add(1, Ordering::SeqCst);
                }
                _ => {}
            }
            HookResult::Allow
        })
        .build();

    let mut bridge = PluginBridge::new();
    let _ = bridge.register(Box::new(p));
    init_bridge(&mut bridge);

    let ctx = build_context(80, 24, "dark");
    bridge.dispatch_input("ls", &ctx);
    bridge.dispatch_output("output\n", &ctx);
    bridge.dispatch_input("pwd", &ctx);

    assert_eq!(input_count.load(Ordering::SeqCst), 2);
    assert_eq!(output_count.load(Ordering::SeqCst), 1);
}

// ═════════════════════════════════════════
// 5. Stats tracking
// ═════════════════════════════════════════

#[test]
fn t_stats_track_hooks_called() {
    let mut bridge = PluginBridge::new();
    let deny_p = native("denier")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .on_hook(|_, _| HookResult::Deny)
        .build();
    let _ = bridge.register(Box::new(deny_p));
    init_bridge(&mut bridge);

    let ctx = build_context(80, 24, "dark");
    bridge.dispatch_input("cmd1", &ctx);
    bridge.dispatch_input("cmd2", &ctx);
    bridge.dispatch_input("cmd3", &ctx);

    let stats = bridge
        .manager()
        .stats_of("denier")
        .expect("plugin should exist");
    assert_eq!(stats.hooks_called, 3);
    assert_eq!(stats.denials, 3);
}

#[test]
fn t_stats_track_transforms() {
    let mut bridge = PluginBridge::new();
    let transform_p = native("transformer")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .on_hook(|_, _| HookResult::Transform("safe".to_string()))
        .build();
    let _ = bridge.register(Box::new(transform_p));
    init_bridge(&mut bridge);

    let ctx = build_context(80, 24, "dark");
    bridge.dispatch_input("test", &ctx);
    bridge.dispatch_input("test", &ctx);

    let stats = bridge
        .manager()
        .stats_of("transformer")
        .expect("plugin should exist");
    assert_eq!(stats.hooks_called, 2);
    assert_eq!(stats.transforms, 2);
}

// ═════════════════════════════════════════
// 6. Context propagation
// ═════════════════════════════════════════

#[test]
fn t_context_fields_propagated() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let cols_seen = Arc::new(AtomicUsize::new(0));
    let rows_seen = Arc::new(AtomicUsize::new(0));
    let theme_seen = Arc::new(std::sync::Mutex::new(String::new()));

    let cs = cols_seen.clone();
    let rs = rows_seen.clone();
    let ts = theme_seen.clone();

    let p = native("ctx-plugin")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .on_hook(move |_, ctx| {
            cs.store(ctx.cols, Ordering::SeqCst);
            rs.store(ctx.rows, Ordering::SeqCst);
            *ts.lock().unwrap() = ctx.theme_name.clone();
            HookResult::Allow
        })
        .build();

    let mut bridge = PluginBridge::new();
    let _ = bridge.register(Box::new(p));
    init_bridge(&mut bridge);

    let ctx = build_context(120, 40, "dracula");
    bridge.dispatch_input("test", &ctx);

    assert_eq!(cols_seen.load(Ordering::SeqCst), 120);
    assert_eq!(rows_seen.load(Ordering::SeqCst), 40);
    assert_eq!(theme_seen.lock().unwrap().as_str(), "dracula");
}

// ═════════════════════════════════════════
// 7. Empty / edge cases
// ═════════════════════════════════════════

#[test]
fn t_empty_bridge_dispatch_allow() {
    let mut bridge = PluginBridge::new();
    let ctx = build_context(80, 24, "dark");
    let result = bridge.dispatch_input("test", &ctx);
    assert_eq!(result, HookResult::Allow);
}

#[test]
fn t_unregister_nonexistent_no_panic() {
    let mut bridge = PluginBridge::new();
    bridge.unregister("does-not-exist");
    assert_eq!(bridge.count(), 0);
}

#[test]
fn t_register_duplicate_name_fails() {
    let mut bridge = PluginBridge::new();
    let p1 = native("dup")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .build();
    let p2 = native("dup")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .build();
    assert!(bridge.register(Box::new(p1)).is_ok());
    assert!(bridge.register(Box::new(p2)).is_err());
}

#[test]
fn t_bridge_from_manager_preserves_plugins() {
    use ggterm_plugin::PluginManager;
    let mut mgr = PluginManager::new();
    let p = native("preserve")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .build();
    let _ = mgr.register(Box::new(p));
    assert_eq!(mgr.count(), 1);

    let mut bridge = PluginBridge::from_manager(mgr);
    assert_eq!(bridge.count(), 1);

    // Verify the plugin still works
    init_bridge(&mut bridge);
    let ctx = build_context(80, 24, "dark");
    let result = bridge.dispatch_input("test", &ctx);
    assert_eq!(result, HookResult::Allow);
}

// ═════════════════════════════════════════
// 8. Real-world scenarios
// ═════════════════════════════════════════

#[test]
fn t_scenario_command_filter() {
    // Simulates a real command-blocking plugin:
    // - Blocks "rm -rf" commands
    // - Allows everything else
    let mut bridge = PluginBridge::new();

    let blocker = native("cmd-blocker")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .on_hook(|hook, _| {
            if let Hook::OnInput(text) = hook
                && text.contains("rm -rf")
            {
                return HookResult::Deny;
            }
            HookResult::Allow
        })
        .build();
    let _ = bridge.register(Box::new(blocker));
    init_bridge(&mut bridge);

    let ctx = build_context(80, 24, "dark");

    // Safe commands pass
    assert_eq!(bridge.dispatch_input("ls", &ctx), HookResult::Allow);
    assert_eq!(
        bridge.dispatch_input("cat file.txt", &ctx),
        HookResult::Allow
    );

    // Dangerous command blocked
    assert_eq!(bridge.dispatch_input("rm -rf /", &ctx), HookResult::Deny);
}

#[test]
fn t_scenario_input_transformer() {
    // Simulates an autocorrect plugin:
    // - Replaces "pyhton" with "python"
    // - Transforms input text
    let mut bridge = PluginBridge::new();

    let autocorrect = native("autocorrect")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .on_hook(|hook, _| {
            if let Hook::OnInput(text) = hook {
                let corrected = text.replace("pyhton", "python");
                if corrected != *text {
                    return HookResult::Transform(corrected);
                }
            }
            HookResult::Allow
        })
        .build();
    let _ = bridge.register(Box::new(autocorrect));
    init_bridge(&mut bridge);

    let ctx = build_context(80, 24, "dark");

    // No typo → Allow
    assert_eq!(bridge.dispatch_input("echo hello", &ctx), HookResult::Allow);

    // Typo → Transform
    let result = bridge.dispatch_input("pyhton script.py", &ctx);
    assert_eq!(
        result,
        HookResult::Transform("python script.py".to_string())
    );
}

#[test]
fn t_scenario_three_plugin_pipeline() {
    // Three plugins:
    // 1. Logger: counts all inputs (Allow)
    // 2. Blocker: blocks "shutdown" (Deny)
    // 3. Lowercase: transforms to lowercase (Transform)
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let log_count = Arc::new(AtomicUsize::new(0));
    let lc = log_count.clone();

    let logger = native("logger")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .on_hook(move |_, _| {
            lc.fetch_add(1, Ordering::SeqCst);
            HookResult::Allow
        })
        .build();

    let blocker = native("blocker")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .on_hook(|hook, _| {
            if let Hook::OnInput(text) = hook
                && text.contains("shutdown")
            {
                return HookResult::Deny;
            }
            HookResult::Allow
        })
        .build();

    let lowercaser = native("lowercaser")
        .version("1.0.0")
        .hook(HookType::OnInput)
        .on_hook(|hook, _| {
            if let Hook::OnInput(text) = hook {
                return HookResult::Transform(text.to_lowercase());
            }
            HookResult::Allow
        })
        .build();

    let mut bridge = PluginBridge::new();
    let _ = bridge.register(Box::new(logger));
    let _ = bridge.register(Box::new(blocker));
    let _ = bridge.register(Box::new(lowercaser));
    init_bridge(&mut bridge);

    let ctx = build_context(80, 24, "dark");

    // Safe command: Logger(Allow) + Blocker(Allow) + Lowercaser(Transform)
    let result = bridge.dispatch_input("LS -LA", &ctx);
    assert_eq!(result, HookResult::Transform("ls -la".to_string()));
    assert_eq!(log_count.load(Ordering::SeqCst), 1);

    // Dangerous: Logger(Allow) + Blocker(Deny) → Deny wins
    let result = bridge.dispatch_input("shutdown now", &ctx);
    assert_eq!(result, HookResult::Deny);
    assert_eq!(log_count.load(Ordering::SeqCst), 2);
}

// ═════════════════════════════════════════
// 9. Lua plugin integration (feature-gated)
// ═════════════════════════════════════════

#[cfg(feature = "plugin-lua")]
mod lua_tests {
    use super::*;
    use ggterm_plugin::LuaPlugin;

    fn init_bridge(bridge: &mut PluginBridge) {
        let ctx = build_context(80, 24, "dark");
        let _ = bridge.init_all(&ctx);
    }

    #[test]
    fn t_lua_allow_input() {
        let source = r#"
            return {
                name = "lua-allow",
                version = "1.0.0",
                hooks = { "on_input" },
                on_input = function(text)
                    return "allow"
                end
            }
        "#;

        let p = LuaPlugin::from_source(source).expect("lua plugin should parse");
        let mut bridge = PluginBridge::new();
        let _ = bridge.register(Box::new(p));
        init_bridge(&mut bridge);

        let ctx = build_context(80, 24, "dark");
        let result = bridge.dispatch_input("ls", &ctx);
        assert_eq!(result, HookResult::Allow);
    }

    #[test]
    fn t_lua_deny_input() {
        let source = r#"
            return {
                name = "lua-deny",
                version = "1.0.0",
                hooks = { "on_input" },
                on_input = function(text)
                    return "deny"
                end
            }
        "#;

        let p = LuaPlugin::from_source(source).expect("lua plugin should parse");
        let mut bridge = PluginBridge::new();
        let _ = bridge.register(Box::new(p));
        init_bridge(&mut bridge);

        let ctx = build_context(80, 24, "dark");
        let result = bridge.dispatch_input("rm -rf /", &ctx);
        assert_eq!(result, HookResult::Deny);
    }

    #[test]
    fn t_lua_transform_input() {
        let source = r#"
            return {
                name = "lua-transform",
                version = "1.0.0",
                hooks = { "on_input" },
                on_input = function(text)
                    return "transform", string.upper(text)
                end
            }
        "#;

        let p = LuaPlugin::from_source(source).expect("lua plugin should parse");
        let mut bridge = PluginBridge::new();
        let _ = bridge.register(Box::new(p));
        init_bridge(&mut bridge);

        let ctx = build_context(80, 24, "dark");
        let result = bridge.dispatch_input("hello", &ctx);
        assert_eq!(result, HookResult::Transform("HELLO".to_string()));
    }

    #[test]
    fn t_lua_mixed_with_native() {
        // Native Allow + Lua Deny → Deny
        let native_allow = native("native-ok")
            .version("1.0.0")
            .hook(HookType::OnInput)
            .on_hook(|_, _| HookResult::Allow)
            .build();

        let lua_source = r#"
            return {
                name = "lua-block",
                version = "1.0.0",
                hooks = { "on_input" },
                on_input = function(text)
                    return "deny"
                end
            }
        "#;

        let lua_deny = LuaPlugin::from_source(lua_source).expect("lua plugin should parse");

        let mut bridge = PluginBridge::new();
        let _ = bridge.register(Box::new(native_allow));
        let _ = bridge.register(Box::new(lua_deny));
        init_bridge(&mut bridge);

        let ctx = build_context(80, 24, "dark");
        let result = bridge.dispatch_input("test", &ctx);
        assert_eq!(result, HookResult::Deny);
    }

    #[test]
    fn t_lua_output_dispatch() {
        let source = r#"
            return {
                name = "lua-output",
                version = "1.0.0",
                hooks = { "on_output" },
                on_output = function(text)
                    return "allow"
                end
            }
        "#;

        let p = LuaPlugin::from_source(source).expect("lua plugin should parse");
        let mut bridge = PluginBridge::new();
        let _ = bridge.register(Box::new(p));
        init_bridge(&mut bridge);

        let ctx = build_context(80, 24, "dark");
        // Should not panic
        bridge.dispatch_output("hello world\n", &ctx);
    }

    #[test]
    fn t_lua_conditional_block() {
        // Lua plugin that blocks specific commands
        let source = r#"
            return {
                name = "lua-guard",
                version = "1.0.0",
                hooks = { "on_input" },
                on_input = function(text)
                    if string.find(text, "shutdown") then
                        return "deny"
                    end
                    if string.find(text, "reboot") then
                        return "deny"
                    end
                    return "allow"
                end
            }
        "#;

        let p = LuaPlugin::from_source(source).expect("lua plugin should parse");
        let mut bridge = PluginBridge::new();
        let _ = bridge.register(Box::new(p));
        init_bridge(&mut bridge);

        let ctx = build_context(80, 24, "dark");

        // Safe command
        assert_eq!(bridge.dispatch_input("ls -la", &ctx), HookResult::Allow);

        // Blocked
        assert_eq!(
            bridge.dispatch_input("shutdown -h now", &ctx),
            HookResult::Deny
        );
        assert_eq!(bridge.dispatch_input("reboot", &ctx), HookResult::Deny);
    }
}
