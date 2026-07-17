//! Build script — embeds git commit hash and build date into the binary.
//!
//! Sets GIT_HASH and BUILD_DATE environment variables at compile time,
//! which version_info.rs reads via option_env!().

fn main() {
    // Git commit hash (short).
    let git_hash = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=GIT_HASH={git_hash}");

    // Build date (ISO format). Cross-platform: use chrono-like approach
    // without external deps by checking system time directly.
    let build_date = build_date_string();
    println!("cargo:rustc-env=BUILD_DATE={build_date}");

    // Re-run if HEAD changes.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
}

/// Get the current date as YYYY-MM-DD without external dependencies.
/// Works on all platforms (no shell command needed).
fn build_date_string() -> String {
    // Use std::time to get epoch seconds, then compute date manually.
    // This avoids platform-specific `date` commands.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Days since epoch.
    let days = now / 86400;

    // Convert days since 1970-01-01 to year/month/day.
    // Algorithm from Howard Hinnant's date algorithms (civil_from_days).
    let z = days as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02}")
}
