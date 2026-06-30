//! P28-G: Sound and bell audio feedback.
//!
//! Plays system sounds for terminal BEL (0x07) and other events.
//! Uses platform-native audio APIs (no external dependencies).

use std::sync::mpsc;
use std::thread;

/// Sound type for different events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundType {
    /// Terminal BEL — default system beep.
    Bell,
    /// Command completed successfully (exit 0).
    Success,
    /// Command failed (non-zero exit).
    Error,
    /// Tab switch click.
    TabSwitch,
    /// New tab opened.
    TabOpen,
    /// Tab closed.
    TabClose,
    /// Split created.
    Split,
    /// Search match found.
    SearchHit,
    /// Custom notification.
    Notify,
}

impl SoundType {
    /// Get the system sound name for macOS.
    #[cfg(target_os = "macos")]
    fn macos_sound(self) -> &'static str {
        match self {
            SoundType::Bell => "Tink",
            SoundType::Success => "Glass",
            SoundType::Error => "Basso",
            SoundType::TabSwitch => "Pop",
            SoundType::TabOpen => "Morse",
            SoundType::TabClose => "Submarine",
            SoundType::Split => "Tink",
            SoundType::SearchHit => "Pop",
            SoundType::Notify => "Hero",
        }
    }

    /// Get a description string (for logging on non-macOS).
    #[allow(dead_code)]
    fn description(self) -> &'static str {
        match self {
            SoundType::Bell => "bell",
            SoundType::Success => "success",
            SoundType::Error => "error",
            SoundType::TabSwitch => "tab-switch",
            SoundType::TabOpen => "tab-open",
            SoundType::TabClose => "tab-close",
            SoundType::Split => "split",
            SoundType::SearchHit => "search-hit",
            SoundType::Notify => "notify",
        }
    }
}

/// Audio player state.
#[derive(Debug, Default)]
pub struct SoundPlayer {
    /// Whether sound is enabled.
    enabled: bool,
    /// Channel to send play commands to the audio thread.
    tx: Option<mpsc::Sender<SoundType>>,
    /// Whether the audio thread is running.
    running: bool,
}

impl SoundPlayer {
    /// Create a new sound player (disabled by default).
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable or disable sound.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if enabled && !self.running {
            self.start_thread();
        } else if !enabled && self.running {
            // Can't easily stop the thread; it's fine, it just won't receive messages
            self.running = false;
        }
    }

    /// Whether sound is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Start the audio playback thread.
    fn start_thread(&mut self) {
        let (tx, rx) = mpsc::channel::<SoundType>();
        self.tx = Some(tx);
        self.running = true;

        thread::spawn(move || {
            for sound in rx {
                play_system_sound(sound);
            }
        });
    }

    /// Play a sound (non-blocking, returns immediately).
    pub fn play(&self, sound: SoundType) {
        if !self.enabled {
            return;
        }
        if let Some(tx) = &self.tx {
            let _ = tx.send(sound);
        }
    }

    /// Toggle sound on/off.
    pub fn toggle(&mut self) {
        self.set_enabled(!self.enabled);
    }
}

/// Play a system sound (blocking).
fn play_system_sound(sound: SoundType) {
    #[cfg(target_os = "macos")]
    {
        let name = sound.macos_sound();
        // Use afplay or the system sound via NSSound
        // We use `afplay /System/Library/Sounds/<name>.aiff` as a simple approach
        let path = format!("/System/Library/Sounds/{}.aiff", name);
        let _ = std::process::Command::new("afplay")
            .arg(&path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }

    #[cfg(target_os = "linux")]
    {
        // Try paplay or aplay with a beep
        let _ = std::process::Command::new("paplay")
            .arg("/usr/share/sounds/freedesktop/stereo/bell.oga")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }

    #[cfg(target_os = "windows")]
    {
        // Use PowerShell to play a system sound
        let _ = std::process::Command::new("powershell")
            .args(["-c", "[console]::beep(800, 200)"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        log::debug!(
            "sound playback not supported on this platform: {}",
            sound.description()
        );
    }
}

/// Bell sound configuration.
#[derive(Debug, Clone)]
pub struct BellConfig {
    /// Whether to play sound on BEL.
    pub sound_on_bell: bool,
    /// Whether to also show visual bell.
    pub visual_on_bell: bool,
    /// Minimum interval between bell sounds (ms) to prevent spam.
    pub min_interval_ms: u64,
    /// Sound type to use for bell.
    pub bell_sound: SoundType,
}

impl Default for BellConfig {
    fn default() -> Self {
        Self {
            sound_on_bell: true,
            visual_on_bell: true,
            min_interval_ms: 300,
            bell_sound: SoundType::Bell,
        }
    }
}

/// Rate limiter for bell sounds.
#[derive(Debug)]
pub struct BellRateLimiter {
    config: BellConfig,
    last_bell: Option<std::time::Instant>,
    /// Count of suppressed bells (for logging).
    suppressed: u64,
}

impl BellRateLimiter {
    /// Create new rate limiter.
    pub fn new(config: BellConfig) -> Self {
        Self {
            config,
            last_bell: None,
            suppressed: 0,
        }
    }

    /// Check if a bell should be played (respects rate limit).
    /// Returns true if the bell should be played, false if suppressed.
    pub fn check(&mut self) -> bool {
        let now = std::time::Instant::now();
        if let Some(last) = self.last_bell {
            let elapsed = now.duration_since(last).as_millis() as u64;
            if elapsed < self.config.min_interval_ms {
                self.suppressed += 1;
                return false;
            }
        }
        self.last_bell = Some(now);
        true
    }

    /// Number of bells suppressed by rate limiting.
    pub fn suppressed_count(&self) -> u64 {
        self.suppressed
    }

    /// Update the config.
    pub fn set_config(&mut self, config: BellConfig) {
        self.config = config;
    }

    /// Get the current config.
    pub fn config(&self) -> &BellConfig {
        &self.config
    }
}

impl Default for BellRateLimiter {
    fn default() -> Self {
        Self::new(BellConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_sound_type_description() {
        assert_eq!(SoundType::Bell.description(), "bell");
        assert_eq!(SoundType::Success.description(), "success");
        assert_eq!(SoundType::Error.description(), "error");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn t_sound_type_macos_sound() {
        assert_eq!(SoundType::Bell.macos_sound(), "Tink");
        assert_eq!(SoundType::Error.macos_sound(), "Basso");
        assert_eq!(SoundType::Success.macos_sound(), "Glass");
    }

    #[test]
    fn t_sound_player_default_disabled() {
        let player = SoundPlayer::new();
        assert!(!player.is_enabled());
    }

    #[test]
    fn t_sound_player_enable() {
        let mut player = SoundPlayer::new();
        player.set_enabled(true);
        assert!(player.is_enabled());
    }

    #[test]
    fn t_sound_player_toggle() {
        let mut player = SoundPlayer::new();
        assert!(!player.is_enabled());
        player.toggle();
        assert!(player.is_enabled());
        player.toggle();
        assert!(!player.is_enabled());
    }

    #[test]
    fn t_sound_player_play_when_disabled() {
        let player = SoundPlayer::new();
        // Should be a no-op, not panic
        player.play(SoundType::Bell);
    }

    #[test]
    fn t_bell_config_default() {
        let config = BellConfig::default();
        assert!(config.sound_on_bell);
        assert!(config.visual_on_bell);
        assert!(config.min_interval_ms > 0);
    }

    #[test]
    fn t_bell_rate_limiter_first_allowed() {
        let mut limiter = BellRateLimiter::default();
        assert!(limiter.check());
    }

    #[test]
    fn t_bell_rate_limiter_suppresses_rapid() {
        let mut limiter = BellRateLimiter::new(BellConfig {
            min_interval_ms: 1000, // 1 second
            ..Default::default()
        });
        assert!(limiter.check()); // First bell passes
        assert!(!limiter.check()); // Immediate second is suppressed
        assert!(!limiter.check()); // Third also suppressed
        assert_eq!(limiter.suppressed_count(), 2);
    }

    #[test]
    fn t_bell_rate_limiter_allows_after_interval() {
        let mut limiter = BellRateLimiter::new(BellConfig {
            min_interval_ms: 10, // 10ms for testing
            ..Default::default()
        });
        assert!(limiter.check());
        std::thread::sleep(std::time::Duration::from_millis(15));
        assert!(limiter.check()); // Should pass after interval
    }

    #[test]
    fn t_bell_rate_limiter_suppressed_count() {
        let mut limiter = BellRateLimiter::new(BellConfig {
            min_interval_ms: 1000,
            ..Default::default()
        });
        limiter.check();
        limiter.check();
        limiter.check();
        assert_eq!(limiter.suppressed_count(), 2);
    }

    #[test]
    fn t_bell_rate_limiter_set_config() {
        let mut limiter = BellRateLimiter::default();
        let new_config = BellConfig {
            min_interval_ms: 500,
            sound_on_bell: false,
            ..Default::default()
        };
        limiter.set_config(new_config);
        assert_eq!(limiter.config().min_interval_ms, 500);
        assert!(!limiter.config().sound_on_bell);
    }
}
