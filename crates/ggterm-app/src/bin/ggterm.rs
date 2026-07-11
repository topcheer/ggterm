//! # GGTerm — GPU-accelerated AI-native terminal emulator.
//!
//! ## Usage
//!
//! ```sh
//! # Default 80x24 terminal
//! ggterm
//!
//! # Custom size and shell
//! ggterm --cols 120 --rows 40 --shell /bin/zsh
//!
//! # With specific theme
//! ggterm --theme dracula
//!
//! # Larger font
//! ggterm --font-size 18
//! ```
//!
//! ## Configuration
//!
//! GGTerm reads `~/.ggterm/config.toml` on startup and watches it for
//! changes (with `config-watch` feature). CLI args override config values.

use std::process::ExitCode;

use clap::Parser;
use ggterm_app::{DesktopApp, DesktopConfig};

/// GGTerm — GPU-accelerated AI-native terminal emulator.
#[derive(Parser, Debug)]
#[command(name = "ggterm", version, about, long_about = None)]
struct Cli {
    /// Initial number of columns.
    #[arg(short = 'c', long, default_value_t = 80)]
    cols: u16,

    /// Initial number of rows.
    #[arg(short = 'r', long, default_value_t = 24)]
    rows: u16,

    /// Shell to spawn (default: $SHELL or /bin/sh).
    #[arg(short = 's', long)]
    shell: Option<String>,

    /// Window title.
    #[arg(short = 't', long, default_value = "GGTerm")]
    title: String,

    /// Color theme: dark, light, dracula, solarized-dark, solarized-light,
    /// gruvbox, nord, tokyo-night, catppuccin-mocha.
    #[arg(long, default_value = "dark")]
    theme: String,

    /// Font size in pixels (also sets cell height).
    #[arg(long, default_value_t = 16.0)]
    font_size: f32,

    /// Cell width in pixels.
    #[arg(long, default_value_t = 8.0)]
    cell_width: f32,

    /// Working directory to start the shell in (default: current directory).
    #[arg(short = 'w', long)]
    working_directory: Option<String>,

    /// Path to a custom config file (default: ~/.ggterm/config.toml).
    #[arg(short = 'C', long)]
    config: Option<String>,

    /// Verbosity: -v info, -vv debug, -vvv trace.
    #[arg(short = 'v', long, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Auto-start P2P share on launch (for testing).
    #[arg(long)]
    p2p_share: bool,

    /// Keep the terminal open after the command exits (use with -e).
    /// Shows "[process exited with code N] — press any key to close"
    /// before closing. Like xterm --hold or alacritty --hold.
    #[arg(long)]
    hold: bool,

    /// Start in fullscreen mode (like pressing F11).
    #[arg(long)]
    fullscreen: bool,

    /// Start maximized (fills the screen but keeps window decorations).
    #[arg(long)]
    maximize: bool,

    /// Execute a command instead of launching an interactive shell.
    /// Usage: ggterm -e vim file.txt  OR  ggterm -e htop
    /// All arguments after -e are passed as the command + args.
    #[arg(short = 'e', long = "execute", num_args = 1.., allow_hyphen_values = true)]
    execute: Vec<String>,

    /// Window geometry in X11 format: COLSxROWS[+X+Y].
    /// Examples: 120x40, 80x24+100+50, 100x30-10-10
    /// Overrides --cols/--rows if COLSxROWS is specified.
    #[arg(short = 'g', long)]
    geometry: Option<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Initialize logging based on verbosity.
    let log_level = match cli.verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(log_level))
        .format_timestamp_millis()
        .try_init();

    log::info!(
        "GGTerm starting: {}x{}, shell={:?}, theme={}",
        cli.cols,
        cli.rows,
        cli.shell,
        cli.theme
    );

    // Apply working directory before launching.
    if let Some(ref dir) = cli.working_directory {
        if let Ok(abs) = std::fs::canonicalize(dir) {
            std::env::set_current_dir(&abs)
                .unwrap_or_else(|e| log::warn!("Failed to cd to {dir}: {e}"));
        } else {
            log::warn!("Working directory does not exist: {dir}");
        }
    }

    // Build desktop config from CLI args.
    // Parse --geometry (X11 format: COLSxROWS[+X+Y]).
    let (cols, rows) = if let Some(ref geom) = cli.geometry {
        let parts: Vec<&str> = geom.split(['x', 'X']).collect();
        if parts.len() >= 2 {
            let c: u16 = parts[0].parse().unwrap_or(cli.cols);
            let r: u16 = parts[1]
                .split(['+', '-'])
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(cli.rows);
            (c, r)
        } else {
            (cli.cols, cli.rows)
        }
    } else {
        (cli.cols, cli.rows)
    };

    let mut config = DesktopConfig::default()
        .with_title(&cli.title)
        .with_size(cols, rows)
        .with_cell_size(cli.cell_width, cli.font_size);

    // Override title to include version info on default.
    if cli.title == "GGTerm" {
        config = config.with_title(format!("GGTerm {}", env!("CARGO_PKG_VERSION")));
    }

    // Auto-start P2P share if flag is set.
    if cli.p2p_share {
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(3));
            log::info!("Auto-starting P2P share (CLI --p2p-share)");
            // Write a flag file that about_to_wait checks.
            let _ = std::fs::write("/tmp/ggterm_auto_share", "1");
        });
    }

    // If -e flag is given, set the shell to execute the command directly.
    if !cli.execute.is_empty() {
        // Build "shell -c 'command args'" so the PTY spawns the command
        // via the user's shell, exactly like xterm -e / alacritty -e.
        let cmd = cli.execute.join(" ");
        log::info!("Execute mode (-e): {cmd}");
        // Store in env var that the shell detection reads.
        // SAFETY: single-threaded before app launch.
        unsafe {
            std::env::set_var("GGTERM_EXEC", &cmd);
        }
    }

    // If --hold flag is given, keep terminal open after command exits.
    if cli.hold {
        // SAFETY: single-threaded before app launch.
        unsafe {
            std::env::set_var("GGTERM_HOLD", "1");
        }
        log::info!("Hold mode: terminal will stay open after command exits");
    }

    // If --fullscreen or --maximize, set env vars for the window builder.
    if cli.fullscreen {
        // SAFETY: single-threaded before app launch.
        unsafe {
            std::env::set_var("GGTERM_FULLSCREEN", "1");
        }
    }
    if cli.maximize {
        // SAFETY: single-threaded before app launch.
        unsafe {
            std::env::set_var("GGTERM_MAXIMIZE", "1");
        }
    }

    // If --config flag is given, set env var for the config loader.
    if let Some(ref config_path) = cli.config {
        // SAFETY: single-threaded before app launch.
        unsafe {
            std::env::set_var("GGTERM_CONFIG", config_path);
        }
        log::info!("Using custom config: {config_path}");
    }

    // Launch the terminal.
    match DesktopApp::run(config) {
        Ok(()) => {
            log::info!("GGTerm exited cleanly");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("GGTerm error: {e}");
            log::error!("GGTerm fatal error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Auto-start P2P share (used by --p2p-share flag).
pub fn should_auto_share() -> bool {
    std::env::args().any(|a| a == "--p2p-share")
}
