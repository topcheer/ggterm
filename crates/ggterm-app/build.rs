//! Build script — embeds git commit hash and build date into the binary.
//!
//! Sets GIT_HASH and BUILD_DATE environment variables at compile time,
//! which version_info.rs reads via option_env!().

use std::process::Command;

fn main() {
    // Git commit hash (short).
    let git_hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=GIT_HASH={git_hash}");

    // Build date (ISO format).
    let build_date = Command::new("date")
        .arg("+%Y-%m-%d")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=BUILD_DATE={build_date}");

    // Re-run if HEAD changes.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
}
