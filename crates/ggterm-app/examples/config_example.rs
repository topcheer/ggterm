//! # GGTerm Configuration Example
//!
//! Demonstrates the full `ConfigManager` lifecycle:
//!
//! 1. **Load** config from `~/.ggterm/config.toml` (or defaults)
//! 2. **Print** current configuration values
//! 3. **Write** a temporary config file and load it with `load_from()`
//! 4. **Hot-reload** — modify the file and call `reload()`
//! 5. **on_change callback** — fires automatically on reload
//! 6. **File watching** (requires `config-watch` feature) — `watch()` + `poll_reload()`
//!
//! ## Running
//!
//! ```sh
//! # Default features (no file watching)
//! cargo run --example config_example
//!
//! # With config-watch feature (demonstrates FS watching)
//! cargo run --features config-watch --example config_example
//! ```

use std::path::PathBuf;

#[cfg(feature = "config-watch")]
use std::thread;
#[cfg(feature = "config-watch")]
use std::time::Duration;

use ggterm_app::config::ConfigManager;

// ── Helper ──────────────────────────────────────────────────────────────

fn print_section(title: &str) {
    println!("\n{}", "─".repeat(60));
    println!("  {title}");
    println!("{}", "─".repeat(60));
}

fn print_config(mgr: &ConfigManager) {
    let cfg = mgr.config();
    println!("  theme            = {}", cfg.appearance.theme);
    println!("  font_family      = {}", cfg.appearance.font_family);
    println!("  font_size        = {}", cfg.appearance.font_size);
    println!("  cell_width       = {}", cfg.appearance.cell_width);
    println!("  cell_height      = {}", cfg.appearance.cell_height);
    println!("  scrollback_lines = {}", cfg.terminal.scrollback_lines);
    println!("  shell            = {}", cfg.terminal.shell);
    println!("  ai.enabled       = {}", cfg.ai.enabled);
    println!("  ai.api_endpoint  = {}", cfg.ai.api_endpoint);
    println!("  ai.model         = {}", cfg.ai.model);
    if let Some(path) = mgr.config_path() {
        println!("  config_path      = {}", path.display());
    } else {
        println!("  config_path      = <none>");
    }
}

// ── Main ────────────────────────────────────────────────────────────────

fn main() {
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║         GGTerm Configuration Example                     ║");
    println!("╚══════════════════════════════════════════════════════════╝");

    // ── 1. Load from default location ──────────────────────────
    //
    // ConfigManager::load_default() reads ~/.ggterm/config.toml.
    // If the file doesn't exist, it falls back to Config::default().

    print_section("1. Load from ~/.ggterm/config.toml");
    let mut mgr = match ConfigManager::load_default() {
        Ok(m) => {
            println!("  Loaded successfully.");
            m
        }
        Err(e) => {
            println!("  Load error (using defaults): {e}");
            ConfigManager::new()
        }
    };
    print_config(&mgr);

    // ── 2. Register on_change callback ─────────────────────────
    //
    // The callback fires every time reload() detects a change.
    // This is how your app can react to hot-reloaded config
    // (e.g. re-apply theme, adjust scrollback).

    print_section("2. Register on_change callback");
    mgr.on_change(Box::new(|cfg| {
        println!(
            "  >> on_change fired! New theme: {}, font_size: {}",
            cfg.appearance.theme, cfg.appearance.font_size
        );
    }));
    println!("  Callback registered.");

    // ── 3. Write a temp config file and load_from() ────────────
    //
    // This simulates a user placing a custom config.toml at
    // an arbitrary path.

    print_section("3. Load from a custom path");

    let temp_dir = std::env::temp_dir();
    let config_path: PathBuf = temp_dir.join("ggterm_config_example.toml");

    let initial_toml = r#"
[appearance]
theme = "solarized"
font_family = "JetBrains Mono"
font_size = 16
cell_width = 9
cell_height = 18

[terminal]
scrollback_lines = 50000
shell = "/bin/zsh"

[ai]
enabled = true
api_endpoint = "https://open.bigmodel.cn/api/paas/v4"
model = "glm-4-flash"
"#;

    std::fs::write(&config_path, initial_toml).expect("failed to write temp config");
    println!("  Wrote temp config to: {}", config_path.display());

    mgr = ConfigManager::load_from(&config_path).expect("load_from failed");
    // Re-register callback (load_from creates a fresh manager).
    mgr.on_change(Box::new(|cfg| {
        println!(
            "  >> on_change fired! New theme: {}, font_size: {}",
            cfg.appearance.theme, cfg.appearance.font_size
        );
    }));
    print_config(&mgr);

    // ── 4. Hot-reload after modifying the file ─────────────────
    //
    // Simulate a user editing config.toml while GGTerm is running.
    // We overwrite the file with new values, then call reload().

    print_section("4. Hot-reload after file modification");

    let updated_toml = r#"
[appearance]
theme = "dracula"
font_family = "Fira Code"
font_size = 20
cell_width = 10
cell_height = 20

[terminal]
scrollback_lines = 100000
shell = "/usr/bin/fish"

[ai]
enabled = false
api_endpoint = "https://api.openai.com/v1"
model = "gpt-4o"
"#;

    std::fs::write(&config_path, updated_toml).expect("failed to write updated config");
    println!("  File modified. Calling reload()...");

    match mgr.reload() {
        Ok(true) => {
            println!("  Config changed! Values updated:");
        }
        Ok(false) => {
            println!("  Config unchanged (same values).");
        }
        Err(e) => {
            eprintln!("  Reload error: {e}");
        }
    }
    print_config(&mgr);

    // ── 5. reload() with no changes ────────────────────────────
    //
    // Calling reload() again without modifying the file
    // returns Ok(false) — the values are identical.

    print_section("5. Reload with no changes → Ok(false)");
    match mgr.reload() {
        Ok(false) => println!("  Correctly detected no changes."),
        Ok(true) => println!("  ERROR: should have been unchanged!"),
        Err(e) => eprintln!("  Unexpected error: {e}"),
    }

    // ── 6. File system watching (config-watch feature) ─────────
    //
    // With the `config-watch` feature, ConfigManager uses the
    // `notify` crate to watch the config file. The watcher runs
    // on a background thread and sets an AtomicBool flag. Your
    // event loop calls poll_reload() to check and reload.

    #[cfg(feature = "config-watch")]
    {
        print_section("6. File watching (config-watch feature)");

        // Start watching.
        println!("  Starting watcher...");
        mgr.watch().expect("watch failed");
        println!("  is_watching = {}", mgr.is_watching());

        // Drain any initial events from file creation.
        thread::sleep(Duration::from_millis(300));
        let _ = mgr.poll_reload();

        // Modify the file — the watcher should detect it.
        let watched_toml = r#"
[appearance]
theme = "light"
font_size = 12

[terminal]
scrollback_lines = 5000
"#;
        std::fs::write(&config_path, watched_toml).expect("failed to write watched config");
        println!("  File modified. Waiting for watcher...");

        // Give the watcher time to fire.
        thread::sleep(Duration::from_millis(500));

        // poll_reload checks the flag and reloads if set.
        match mgr.poll_reload() {
            Ok(true) => {
                println!("  poll_reload detected the change!");
                println!("  New theme: {}", mgr.config().appearance.theme);
            }
            Ok(false) => {
                println!("  poll_reload: no change detected (timing-dependent).");
            }
            Err(e) => eprintln!("  poll_reload error: {e}"),
        }

        // Stop watching.
        mgr.stop_watch();
        println!("  is_watching = {} (after stop)", mgr.is_watching());
    }

    #[cfg(not(feature = "config-watch"))]
    {
        print_section("6. File watching (skipped — run with --features config-watch)");
        println!("  To see file system watching in action, run:");
        println!("    cargo run --features config-watch --example config_example");
    }

    // ── Cleanup ───────────────────────────────────────────────
    let _ = std::fs::remove_file(&config_path);

    println!("\n{}", "─".repeat(60));
    println!("  Done! Temp config cleaned up.");
    println!("{}", "─".repeat(60));
}
